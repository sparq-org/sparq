//! The host/circuit PIN for the `DualLeafV2` reserved no-value sentinel and the
//! scale-free value-lane constants (`zk/dual-leaf-v2/src/lib.nr`).
//!
//! The Noir library carries the V2 sentinel and lane constants as `global`s and
//! re-checks the sentinel's band exclusion in Noir (`nargo test`, run by the
//! `zk-toolchain` lane). A Noir `global` is a hex LITERAL: nothing in the Noir
//! toolchain can re-derive it, so without this file the two sides could drift
//! apart silently — a host derivation change (a new BLAKE3 context string, a
//! changed framing) would leave the circuit mirror stale and still green.
//!
//! This test is the pin. It reads the Noir source with `include_str!` (so a
//! moved or renamed file is a COMPILE error, not a skip), parses each
//! `pub global NAME: Field = <literal>;`, and asserts each one equals its host
//! derivation in [`sparq_zk::dual_leaf_v2`]. It also re-asserts the band
//! exclusion on the value parsed OUT OF THE NOIR FILE, so the literal the
//! circuit would constrain against — not merely the host function — is the one
//! shown to be out of band.
//!
//! The pin changes no proving path: no shipped Noir member imports that library,
//! so nothing here changes which proofs verify. Research-grade, NOT externally
//! audited (sq-qhy4); the value lanes carry the documented INV-VL downgrade
//! (#769 accepted, CR-G8).
#![cfg(feature = "dual-leaf")]

use sparq_zk::dual_leaf::LANG_NONE;
use sparq_zk::dual_leaf_v2;
use sparq_zk::field::{field_from_hex_str, field_to_hex};
use sparq_zk::Fr;

/// The Noir mirror, compiled into this test binary. A path change breaks the
/// build rather than silently disabling the pin.
const NOIR_LIB: &str = include_str!("../../../zk/dual-leaf-v2/src/lib.nr");

/// Every `pub global NAME: Field = <literal>;` declaration in the Noir library,
/// in declaration order.
fn noir_globals() -> Vec<(String, Fr)> {
    // Globals may wrap across lines, so parse the whole file as one string and
    // split on the declaration keyword rather than line by line.
    let mut out = Vec::new();
    for chunk in NOIR_LIB.split("pub global ").skip(1) {
        let (name, rest) = chunk.split_once(':').expect("a global names a type");
        let (_ty, rest) = rest.split_once('=').expect("a global is initialised");
        let literal = rest
            .split_once(';')
            .expect("a global declaration terminates")
            .0
            .split_whitespace()
            .collect::<String>();
        // Hex literals go through the field parser; a decimal literal must be a
        // plain `u64` (only `LANG_NONE = 1` today). Anything else is refused, so
        // a reformatted literal cannot parse to a different value.
        let value = if literal.starts_with("0x") {
            field_from_hex_str(&literal)
        } else if !literal.is_empty() && literal.bytes().all(|b| b.is_ascii_digit()) {
            literal.parse::<u64>().ok().map(Fr::from)
        } else {
            None
        }
        .unwrap_or_else(|| panic!("global {} is not a field literal: {literal}", name.trim()));
        out.push((name.trim().to_string(), value));
    }
    // Every `global` keyword in the file must be one this parser read, so a
    // non-`pub` or differently spaced declaration cannot slip past the pin.
    let declared = NOIR_LIB
        .lines()
        .map(str::trim_start)
        .filter(|l| !l.starts_with("//"))
        .filter(|l| l.starts_with("global ") || l.starts_with("pub global "))
        .count();
    assert_eq!(declared, out.len(), "every Noir global must be a parsed `pub global`");
    out
}

fn noir_global(name: &str) -> Fr {
    noir_globals()
        .into_iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("the Noir library declares no global {}", name))
        .1
}

/// The parser must actually find the declarations — otherwise every assertion
/// below would be vacuous against an empty file.
#[test]
fn the_noir_library_declares_the_pinned_globals() {
    let names: Vec<String> = noir_globals().into_iter().map(|(n, _)| n).collect();
    assert_eq!(
        names,
        vec![
            "HANDLE_BAND_LIMIT",
            "VALUE_NONE_V2",
            "LANG_NONE",
            "INTEGER_LANE_CONST",
            "DOUBLE_LANE_CONST",
            "BOOLEAN_LANE_CONST",
            "DATETIME_LANE_CONST",
            "DATE_LANE_CONST",
        ],
        "the Noir mirror's global inventory changed; pin the new constants here"
    );
}

/// THE PIN. The Noir literal is the host-derived V2 sentinel, byte for byte.
#[test]
fn the_noir_value_none_literal_equals_the_host_sentinel() {
    let host = dual_leaf_v2::value_none();
    assert_eq!(
        field_to_hex(&noir_global("VALUE_NONE_V2")),
        field_to_hex(&host),
        "the Noir VALUE_NONE_V2 literal drifted from dual_leaf_v2::value_none()"
    );
    // The flat IRI / blank-node slot-0 tag is the same reserved element.
    assert_eq!(field_to_hex(&dual_leaf_v2::no_value()), field_to_hex(&host));
}

/// The band exclusion, re-checked on the value the CIRCUIT would carry.
#[test]
fn the_noir_value_none_literal_lies_outside_the_handle_band() {
    assert!(
        !dual_leaf_v2::is_in_handle_band(&noir_global("VALUE_NONE_V2")),
        "the Noir VALUE_NONE_V2 literal lies inside the reachable handle band"
    );
}

/// The band predicate's own edges, so the assertion above is a real exclusion
/// and not a predicate that refuses everything.
#[test]
fn the_handle_band_contains_the_values_a_member_can_range_check() {
    let limit = noir_global("HANDLE_BAND_LIMIT");
    assert_eq!(
        field_to_hex(&limit),
        field_to_hex(&(Fr::from(u64::MAX) + Fr::from(1u64))),
        "the Noir HANDLE_BAND_LIMIT is not 2^64"
    );
    for inside in [Fr::from(0u64), Fr::from(1u64), Fr::from(u64::MAX)] {
        assert!(dual_leaf_v2::is_in_handle_band(&inside));
        assert!(dual_leaf_v2::is_in_handle_band(&-inside));
    }
    assert!(!dual_leaf_v2::is_in_handle_band(&limit));
    assert!(!dual_leaf_v2::is_in_handle_band(&-limit));
}

/// The no-language tag the Noir mirror carries is V1's reserved element, which
/// is what the shipped value-lane members' `LANG_NONE` already is.
#[test]
fn the_noir_lang_none_literal_equals_the_host_tag() {
    assert_eq!(
        field_to_hex(&noir_global("LANG_NONE")),
        field_to_hex(&Fr::from(LANG_NONE))
    );
}

/// Every scale-free lane constant is pinned to its host derivation. The decimal
/// lane is scale-parameterised and deliberately has no Noir global.
#[test]
fn the_noir_lane_constants_equal_their_host_derivations() {
    for (name, host) in [
        ("INTEGER_LANE_CONST", dual_leaf_v2::integer_lane_const()),
        ("DOUBLE_LANE_CONST", dual_leaf_v2::double_lane_const()),
        ("BOOLEAN_LANE_CONST", dual_leaf_v2::boolean_lane_const()),
        ("DATETIME_LANE_CONST", dual_leaf_v2::datetime_lane_const()),
        ("DATE_LANE_CONST", dual_leaf_v2::date_lane_const()),
    ] {
        assert_eq!(
            field_to_hex(&noir_global(name)),
            field_to_hex(&host),
            "the Noir {} literal drifted from its host derivation",
            name
        );
    }
}
