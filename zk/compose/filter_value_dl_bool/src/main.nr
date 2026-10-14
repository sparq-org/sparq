// Circuit-family member: DUAL-LEAF BIT-DOMAIN value-lane FILTER over a committed
// xsd:boolean literal. The dedicated boolean sibling of `filter_value_dl_int`:
// the public layout is byte-for-byte that member's, and it additionally
// constrains BOTH the witnessed VALUE_HOOK and the public `bound` to the bit
// domain {0, 1} -- so a handle of 2, which the integer member's u64 range
// decomposition admits, denotes no boolean value and the relation refuses it.
//
// A separate member on purpose: the shared integer member legitimately uses the
// whole u64 domain, and adding these constraints to it would change its gate
// count and its measured evidence.
//
// Like the other value-lane members it binds the operand to the dual-leaf
// commitment via TWO Poseidon2 permutations and carries `lexical_component` as a
// FREE witness -- NO in-circuit blake3. It carries the DOCUMENTED INV-VL
// downgrade: value<->lexical agreement on the value-FILTER lane is
// TRUSTED-ISSUER-HONESTY, not machine-enforced (#769 accepted, CR-G8 / sq-qhy4).
// The whole ZK estate is NOT externally audited; no soundness/privacy claim.
// Relation: sparq_zk_compose_core::filter_value_bool::filter_value_dl_bool.
use sparq_zk_compose_core::filter_value_bool::filter_value_dl_bool;

fn main(
    // -- public --
    challenge: pub Field,    // verifier nonce (replay binding S2.5)
    operand_enc: pub Field,  // the hidden column's dual-leaf term encoding (scan anchor)
    op: pub u32,             // 0=lt 1=le 2=gt 3=ge 4=eq 5=ne
    bound: pub u64,          // the FILTER's constant operand, constrained to {0, 1}
    datatype_const: pub Field, // the boolean lane's constant, as a field
    expected: pub bool,      // the disclosed verdict for this operand
    // -- private --
    value_hook: Field,        // the boolean value handle, constrained to {0, 1}
    lexical_component: Field,  // OFF-circuit blake3 lexical hash, carried as a witness
) {
    let _ = challenge;
    filter_value_dl_bool(
        operand_enc,
        op,
        bound,
        expected,
        value_hook,
        lexical_component,
        datatype_const,
    );
}
