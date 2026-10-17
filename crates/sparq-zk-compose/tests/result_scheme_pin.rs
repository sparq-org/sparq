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

/// The Noir relations, compiled into this test binary. A path change breaks the
/// build rather than silently disabling the pin.
///
/// `RESULT_M5_NR` is the METHOD-BOUND relation this file pins. `RESULT_NR` and
/// `RESULT_PUBLIC_NR` are the RETAINED method-unbound (`ZKSIG_C3`) relations: 30
/// shipped members compile them and the four `result_v1_*` members are pinned to
/// an immutable verification-key baseline, so they must keep the C3 tag. This
/// file pins BOTH facts.
const RESULT_M5_NR: &str = include_str!("../../../zk/compose/compose_core/src/result_m5.nr");
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
        noir_globals(RESULT_M5_NR)
            .into_iter()
            .map(|(n, _)| n)
            .collect::<Vec<_>>(),
        vec![
            "PRESENTATION_STATUS_SCHEME",
            "STATUS_REF",
            "ACCEPTED_STATUS",
            "SCHEME_STRING_CANONICAL_V1",
        ],
        "result_m5.nr's Field-global inventory changed; pin the new constants here"
    );
    // The retained C3 relations keep their own inventory, unchanged.
    assert_eq!(
        noir_globals(RESULT_NR)
            .into_iter()
            .map(|(n, _)| n)
            .collect::<Vec<_>>(),
        vec!["STATUS_REF", "COMMITMENT_STATUS", "ACCEPTED_STATUS"],
        "result.nr's Field-global inventory changed; it is a retained contract"
    );
    assert_eq!(
        noir_globals(RESULT_PUBLIC_NR)
            .into_iter()
            .map(|(n, _)| n)
            .collect::<Vec<_>>(),
        vec!["STATUS_REF", "COMMITMENT_STATUS", "ACCEPTED_STATUS"],
        "result_public.nr's Field-global inventory changed; it is a retained contract"
    );
}

/// The baked scheme digest is the string-canonical method's, byte for byte.
#[test]
fn the_noir_scheme_constant_equals_the_host_string_canonical_digest() {
    assert_eq!(
        field_to_hex(&noir_global(RESULT_M5_NR, "SCHEME_STRING_CANONICAL_V1")),
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
    let pinned = noir_global(RESULT_M5_NR, "SCHEME_STRING_CANONICAL_V1");
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
    assert_eq!(
        field_to_hex(&noir_global(RESULT_M5_NR, "PRESENTATION_STATUS_SCHEME")),
        field_to_hex(&expected),
        "result_m5.nr folds a tag that is not ZKSIG_M5"
    );
}

/// THE PIN. The message the CIRCUIT recomputes from its own literals is the
/// message the HOST signs and verifies.
///
/// `result_m5.nr` computes
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

    let in_circuit = sparq_zk::poseidon2::hash(&[
        noir_global(RESULT_M5_NR, "PRESENTATION_STATUS_SCHEME"),
        root,
        salt,
        reference,
        noir_global(RESULT_M5_NR, "SCHEME_STRING_CANONICAL_V1"),
    ]);
    assert_eq!(
        field_to_hex(&in_circuit),
        field_to_hex(&host),
        "the message result_m5.nr recomputes in circuit is not the one the host signs"
    );
}

/// The in-circuit message is NOT the legacy method-unbound one, so the pin above
/// is a real change of signed object rather than two names for one message.
#[test]
fn the_circuit_message_is_not_the_method_unbound_message() {
    let root = Fr::from(0x1234_5678_u64);
    let salt = Fr::from(0x9abc_def0_u64);
    let reference = Fr::from(0x0fed_cba9_u64);
    let bound = sparq_zk::poseidon2::hash(&[
        noir_global(RESULT_M5_NR, "PRESENTATION_STATUS_SCHEME"),
        root,
        salt,
        reference,
        noir_global(RESULT_M5_NR, "SCHEME_STRING_CANONICAL_V1"),
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

/// The method-UNBOUND `ZKSIG_C3` tag must be GONE from the METHOD-BOUND relation
/// -- leaving the literal behind would let a later edit recompute the unbound
/// message beside the bound one -- and RETAINED in the two legacy relations,
/// whose 30 shipped members authenticate over exactly that message and four of
/// which are pinned to an immutable verification-key baseline.
#[test]
fn the_unbound_tag_is_gone_from_the_bound_relation_and_kept_in_the_legacy_ones() {
    let c3 = format!("{:#x}", u64::from_be_bytes(*b"ZKSIG_C3"));
    assert!(
        !RESULT_M5_NR.contains(&c3),
        "result_m5.nr carries the method-unbound ZKSIG_C3 domain tag ({})",
        c3
    );
    for (what, source) in [("result.nr", RESULT_NR), ("result_public.nr", RESULT_PUBLIC_NR)] {
        assert!(
            source.contains(&c3),
            "{} lost the method-unbound ZKSIG_C3 domain tag ({}); its members are a retained contract",
            what,
            c3
        );
    }
}

/// THE DRIFT PIN. `result_m5::result_check_m5` is `result::result_check` with
/// exactly one substitution: the credential message. This test performs that
/// substitution on `result.nr`'s text and asserts the output is byte-equal to the
/// function in `result_m5.nr`.
///
/// Why a text derivation and not a shared function: `result_check` is compiled by
/// 30 shipped members, four of which are pinned to an immutable verification-key
/// baseline, so it cannot be refactored to share code with the bound relation
/// without risking their bytes. The duplication is deliberate; this pin is what
/// stops the copies diverging. An edit to either relation that is not mirrored in
/// the other turns it red, and the failure prints which side to update.
#[test]
fn the_bound_relation_is_the_unbound_one_with_only_the_message_substituted() {
    /// The single line `result_check` folds the method-UNBOUND message on.
    const UNBOUND_MESSAGE: &str =
        "let message = h4(COMMITMENT_STATUS, root, salts[g], reference);";
    /// What it becomes in the bound relation.
    const BOUND_MESSAGE: &str = "let message = h5(
            PRESENTATION_STATUS_SCHEME,
            root,
            salts[g],
            reference,
            SCHEME_STRING_CANONICAL_V1,
        );";

    /// The text of one `pub fn <name><...>` through the first line that is
    /// exactly `}`, which is how both relations are terminated.
    fn function(source: &str, signature: &str) -> String {
        let start = source
            .find(signature)
            .unwrap_or_else(|| panic!("no function starting {} in the source", signature));
        let rest = &source[start..];
        let end = rest.find("\n}\n").expect("the function terminates at a bare closing brace");
        rest[..end + 3].to_string()
    }

    let unbound = function(RESULT_NR, "pub fn result_check<");
    let bound = function(RESULT_M5_NR, "pub fn result_check_m5<");

    assert_eq!(
        unbound.matches(UNBOUND_MESSAGE).count(),
        1,
        "result_check no longer folds the unbound message exactly once; re-derive this pin"
    );
    let derived = unbound
        .replacen("pub fn result_check<", "pub fn result_check_m5<", 1)
        .replacen(UNBOUND_MESSAGE, BOUND_MESSAGE, 1);

    assert_eq!(
        derived, bound,
        "result_m5.nr's relation is no longer result.nr's with only the credential \
         message substituted: the two have drifted, and whichever side changed must \
         be mirrored in the other"
    );
}

/// The CONTROL for the derivation pin: the substitution is not a no-op, so the
/// equality above is a statement about two genuinely different relations.
#[test]
fn the_message_substitution_actually_changes_the_relation() {
    assert!(
        RESULT_NR.contains("h4(COMMITMENT_STATUS, root, salts[g], reference)"),
        "result.nr must still fold the unbound message"
    );
    assert!(
        !RESULT_M5_NR.contains("h4(COMMITMENT_STATUS, root, salts[g], reference)"),
        "result_m5.nr must not fold the unbound message"
    );
    assert!(
        RESULT_M5_NR.contains("PRESENTATION_STATUS_SCHEME,"),
        "result_m5.nr must fold the bound message"
    );
}
