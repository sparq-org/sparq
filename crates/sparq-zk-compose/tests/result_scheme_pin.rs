//! The host/circuit PIN for the successful-result relation's METHOD-BOUND
//! credential attestation message (`ZKSIG_M5`).
//!
//! The result relations authenticate every credential slot by recomputing the
//! issuer's signed message IN CIRCUIT and running `schnorr_verify` against it
//! (`zk/compose/compose_core/src/result.nr`, and the v4 public-pattern copy in
//! `result_public.nr`). The message is built from Noir `global` LITERALS: the
//! `ZKSIG_M5` domain tag and the folded `scheme_iri_to_field` digest of the
//! STRING-CANONICAL commitment method's `zk:scheme` IRI. Nothing in the Noir
//! toolchain can re-derive either, so without this file the host prover and the
//! circuit could drift apart silently — a credential the host signed and
//! accepted would simply fail to prove, or, worse, the circuit could keep
//! recomputing a message the host no longer builds.
//!
//! This test is the pin. It reads both Noir sources with `include_str!` (so a
//! moved or renamed file is a COMPILE error, not a skip), parses the `global`
//! declarations out of them, and asserts that the message the CIRCUIT would
//! recompute from those literals is byte-for-byte
//! `sig::presentation_message_with_status_and_scheme(C(G), salt, status_ref,
//! scheme_iri_to_field(StringCanonicalV1))` — the message the host signs and
//! verifies. Reverting either side alone turns this red.
//!
//! It also pins the NEGATIVE half: the method-UNBOUND `ZKSIG_C3` tag the
//! relation used to recompute must no longer appear in either source, so the
//! unbound message cannot be reintroduced alongside the bound one.
//!
//! Research-grade, NOT externally audited (sq-qhy4). Authenticating the
//! commitment method opens no value-lane gate and is not a soundness claim.

use sparq_zk::commit::CommitmentMethod;
use sparq_zk::field::{field_from_hex_str, field_to_hex};
use sparq_zk::sig;
use sparq_zk::Fr;

/// The two Noir relations, compiled into this test binary. A path change breaks
/// the build rather than silently disabling the pin.
const RESULT_NR: &str = include_str!("../../../zk/compose/compose_core/src/result.nr");
const RESULT_PUBLIC_NR: &str =
    include_str!("../../../zk/compose/compose_core/src/result_public.nr");

/// Every `global NAME: Field = <literal>;` declaration in one Noir source
/// (`pub` or not), as `(name, value)` in declaration order.
fn noir_globals(source: &str) -> Vec<(String, Fr)> {
    let mut out = Vec::new();
    // Globals may wrap across lines, so split the whole source on the
    // declaration keyword rather than reading it line by line. Splitting on
    // "global " also matches the "pub global " form, whose `pub` stays behind in
    // the preceding chunk.
    for chunk in source.split("global ").skip(1) {
        let Some((name, rest)) = chunk.split_once(':') else {
            continue;
        };
        let Some((ty, rest)) = rest.split_once('=') else {
            continue;
        };
        // Only `Field` globals carry a signed-message constant; the relations
        // also declare `u32` capacities, which are not part of this pin.
        if ty.trim() != "Field" {
            continue;
        }
        let literal = rest
            .split_once(';')
            .expect("a global declaration terminates")
            .0
            .split_whitespace()
            .collect::<String>();
        let value = if let Some(hex) = literal.strip_prefix("0x") {
            // Reject a malformed hex literal rather than letting the field
            // parser coerce it to something else.
            assert!(
                !hex.is_empty() && hex.bytes().all(|b| b.is_ascii_hexdigit()),
                "global {} is not a hex literal: {literal}",
                name.trim()
            );
            field_from_hex_str(&literal)
        } else if !literal.is_empty() && literal.bytes().all(|b| b.is_ascii_digit()) {
            literal.parse::<u64>().ok().map(Fr::from)
        } else {
            None
        }
        .unwrap_or_else(|| panic!("global {} is not a field literal: {literal}", name.trim()));
        out.push((name.trim().to_string(), value));
    }
    out
}

fn noir_global(source: &str, name: &str) -> Fr {
    noir_globals(source)
        .into_iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("the Noir source declares no Field global {}", name))
        .1
}

/// The host derivation the circuit literal must equal.
fn host_scheme_digest() -> Fr {
    sig::scheme_iri_to_field(CommitmentMethod::StringCanonicalV1.scheme_iri())
}

/// The parser must actually find the declarations — otherwise every assertion
/// below would be vacuous against an empty parse.
#[test]
fn the_noir_relations_declare_the_pinned_field_globals() {
    assert_eq!(
        noir_globals(RESULT_NR)
            .into_iter()
            .map(|(n, _)| n)
            .collect::<Vec<_>>(),
        vec![
            "STATUS_REF",
            "PRESENTATION_STATUS_SCHEME",
            "ACCEPTED_STATUS",
            "SCHEME_STRING_CANONICAL_V1",
        ],
        "result.nr's Field-global inventory changed; pin the new constants here"
    );
    assert_eq!(
        noir_globals(RESULT_PUBLIC_NR)
            .into_iter()
            .map(|(n, _)| n)
            .collect::<Vec<_>>(),
        vec!["STATUS_REF", "PRESENTATION_STATUS_SCHEME", "ACCEPTED_STATUS"],
        "result_public.nr's Field-global inventory changed; pin the new constants here"
    );
}

/// The baked scheme digest is the string-canonical method's, byte for byte.
#[test]
fn the_noir_scheme_constant_equals_the_host_string_canonical_digest() {
    assert_eq!(
        field_to_hex(&noir_global(RESULT_NR, "SCHEME_STRING_CANONICAL_V1")),
        field_to_hex(&host_scheme_digest()),
        "the Noir SCHEME_STRING_CANONICAL_V1 literal drifted from \
         scheme_iri_to_field(StringCanonicalV1.scheme_iri())"
    );
}

/// The baked digest is the string-canonical method's and NO other recognised
/// method's, so the constant genuinely fixes the lane rather than naming one
/// arbitrary element.
#[test]
fn the_noir_scheme_constant_matches_no_other_recognised_method() {
    let pinned = noir_global(RESULT_NR, "SCHEME_STRING_CANONICAL_V1");
    for method in CommitmentMethod::recognised() {
        let digest = sig::scheme_iri_to_field(method.scheme_iri());
        let same = field_to_hex(&digest) == field_to_hex(&pinned);
        assert_eq!(
            same,
            *method == CommitmentMethod::StringCanonicalV1,
            "the pinned scheme digest matches {} but should match only the \
             string-canonical method",
            method.scheme_iri()
        );
    }
}

/// The domain tag the circuit folds is `ZKSIG_M5`, the METHOD-BOUND
/// PRESENTATION tag — not the registry's `ZKSIG_M3`, whose signed statement
/// ("recorded in my registry") is not an authorisation to present.
#[test]
fn the_noir_domain_tag_is_the_presentation_method_bound_tag() {
    let expected = Fr::from(u64::from_be_bytes(*b"ZKSIG_M5"));
    for (what, source) in [("result.nr", RESULT_NR), ("result_public.nr", RESULT_PUBLIC_NR)] {
        assert_eq!(
            field_to_hex(&noir_global(source, "PRESENTATION_STATUS_SCHEME")),
            field_to_hex(&expected),
            "{} folds a tag that is not ZKSIG_M5",
            what
        );
    }
}

/// THE PIN. The message the CIRCUIT recomputes from its own literals is the
/// message the HOST signs and verifies.
///
/// `result.nr` computes
/// `h5(PRESENTATION_STATUS_SCHEME, root, salt, reference, SCHEME_STRING_CANONICAL_V1)`
/// with `h5 = Poseidon2::hash(.., 5)`, which is `sparq_zk::poseidon2::hash` over
/// the same five elements. Recomputing it here from the PARSED literals and
/// comparing against `sig::presentation_message_with_status_and_scheme` closes
/// the host/circuit seam: reverting either side alone turns this red.
#[test]
fn the_circuit_recomputed_message_equals_the_host_signed_message() {
    // Stand-in values for the three witnessed/recomputed slots. Their actual
    // provenance is irrelevant to the pin: what is pinned is that both sides
    // fold the SAME five elements in the SAME order under the same tag.
    let root = Fr::from(0x1234_5678_u64);
    let salt = Fr::from(0x9abc_def0_u64);
    let reference = Fr::from(0x0fed_cba9_u64);

    let host = sig::presentation_message_with_status_and_scheme(
        &root,
        &salt,
        &reference,
        &host_scheme_digest(),
    );

    for (what, source, scheme_source) in [
        ("result.nr", RESULT_NR, RESULT_NR),
        // The v4 relation imports the constant from `result.nr` and declares
        // only the tag, so its scheme digest comes from the same literal.
        ("result_public.nr", RESULT_PUBLIC_NR, RESULT_NR),
    ] {
        let in_circuit = sparq_zk::poseidon2::hash(&[
            noir_global(source, "PRESENTATION_STATUS_SCHEME"),
            root,
            salt,
            reference,
            noir_global(scheme_source, "SCHEME_STRING_CANONICAL_V1"),
        ]);
        assert_eq!(
            field_to_hex(&in_circuit),
            field_to_hex(&host),
            "the message {} recomputes in circuit is not the one the host signs",
            what
        );
    }
}

/// The in-circuit message is NOT the legacy method-unbound one, so the pin above
/// is a real change of signed object rather than two names for one message.
#[test]
fn the_circuit_message_is_not_the_method_unbound_message() {
    let root = Fr::from(0x1234_5678_u64);
    let salt = Fr::from(0x9abc_def0_u64);
    let reference = Fr::from(0x0fed_cba9_u64);
    let bound = sparq_zk::poseidon2::hash(&[
        noir_global(RESULT_NR, "PRESENTATION_STATUS_SCHEME"),
        root,
        salt,
        reference,
        noir_global(RESULT_NR, "SCHEME_STRING_CANONICAL_V1"),
    ]);
    let unbound = sig::commitment_message_with_status(&root, &salt, &reference);
    assert_ne!(field_to_hex(&bound), field_to_hex(&unbound));
    // And not the REGISTRY statement's `ZKSIG_M3` message either.
    let registry = sig::commitment_message_with_status_and_scheme(
        &root,
        &salt,
        &reference,
        &host_scheme_digest(),
    );
    assert_ne!(
        field_to_hex(&bound),
        field_to_hex(&registry),
        "the relation authenticates over the registry's signed object"
    );
}

/// The method-UNBOUND `ZKSIG_C3` tag must be GONE from both relations: leaving
/// the literal behind would let a later edit recompute the unbound message
/// beside the bound one.
#[test]
fn the_method_unbound_tag_is_absent_from_both_relations() {
    let c3 = format!("{:#x}", u64::from_be_bytes(*b"ZKSIG_C3"));
    for (what, source) in [("result.nr", RESULT_NR), ("result_public.nr", RESULT_PUBLIC_NR)] {
        assert!(
            !source.contains(&c3),
            "{} still carries the method-unbound ZKSIG_C3 domain tag ({})",
            what,
            c3
        );
    }
}
