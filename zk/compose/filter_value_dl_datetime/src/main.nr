// Circuit-family member: DUAL-LEAF value-lane FILTER over a committed
// xsd:dateTime OR xsd:date (sq-wz99x; research/zk-field-native-encoding.md
// S13.5 -- the circuit half of the S13 dateTime/date lane whose host encoder is
// sq-we9vs). Structurally the filter_value_dl_decimal member with a different
// value handle: the VALUE_HOOK is the SIGNED SCALED EPOCH (milliseconds from
// 1970-01-01T00:00:00Z on the XSD proleptic-Gregorian timeOnTimeline, lane-fixed
// FS=3), carried sign-split, bound to the dual-leaf commitment via TWO Poseidon2
// permutations with `lexical_component` as a FREE witness -- NO in-circuit
// blake3 -- and compared by the UNCHANGED signed_scaled_verdict.
//
// ONE relation, BOTH lanes: xsd:date is its own lane purely by its PUBLIC
// datatype_const (the host picks blake3("<IRI>@epochscale=3") for dateTime or
// for date, sparq_zk::dual_leaf_datetime::{datetime_datatype_const,
// date_datatype_const}). That constant is folded into value_component, so a date
// leaf's honest witness rebinds to a DIFFERENT leaf under the dateTime constant
// and fails the leaf assert -- even though a date's hook is the scaled epoch of
// its STARTING instant and is therefore numerically equal to the dateTime hook of
// that same instant. No second Noir fn is needed (S13.5). This is a BINDING
// argument resting on Poseidon2 preimage resistance, NOT an audited soundness
// claim.
//
// The hookable DOMAIN (strict XSD-canonical Z-timezoned lexicals only; bare /
// non-Z-offset / 24:00:00 / leap-second / non-canonical-year / over-FS-fraction /
// u64-overflowing forms REJECTED) is the host's S13.4 fail-closed predicate, not
// an in-circuit check -- the same division of labour the decimal member uses for
// its canonical fraction width. The member is FS-AGNOSTIC in the same sense the
// decimal member is scale-agnostic: the scale lives in the public datatype_const,
// so a higher-FS lane is a compatible future addition with no new package.
//
// Carries the DOCUMENTED INV-VL downgrade: value<->lexical agreement is
// TRUSTED-ISSUER-HONESTY, not machine-enforced (#769 accepted, CR-G8 / sq-qhy4).
// The S13 rule set is itself an OPEN external-audit obligation. The ZK estate is
// NOT externally audited; no soundness/privacy claim.
// Relation: sparq_zk_compose_core::filter_value::filter_value_dl_datetime.
use sparq_zk_compose_core::filter_value::filter_value_dl_datetime;

fn main(
    // -- public --
    challenge: pub Field,        // verifier nonce (replay binding S2.5)
    operand_enc: pub Field,      // the hidden column's dual-leaf dateTime/date encoding (scan anchor)
    op: pub u32,                 // 0=lt 1=le 2=gt 3=ge 4=eq 5=ne
    bound_neg: pub bool,         // sign of the FILTER's constant instant (pre-epoch)
    bound_scaled_epoch: pub u64, // |T_bound| in milliseconds (host-converted to the lane scale)
    datatype_const: pub Field,   // blake3("<xsd:dateTime|xsd:date IRI>@epochscale=3") -- selects the lane
    expected: pub bool,          // the disclosed verdict for this operand
    // -- private --
    value_neg: bool,             // sign of the operand's epoch (pre-epoch instant)
    value_hook_scaled: Field,    // |T| in milliseconds -- the value handle at the lane scale
    lexical_component: Field,    // OFF-circuit blake3 lexical hash, carried as a witness
) {
    let _ = challenge;
    filter_value_dl_datetime(
        operand_enc,
        op,
        bound_neg,
        bound_scaled_epoch,
        expected,
        value_neg,
        value_hook_scaled,
        lexical_component,
        datatype_const,
    );
}
