// Everything but the BLAKE3 opening. blake3_leaf.py appends hand-written
// ACIR that constrains each hs[i] to the token of values[i].
use sparq_zk_compose_core::hashes::{h2, TYPE_CODE_LITERAL};

fn main(
    operands: pub [Field; leaf::N],
    ops: pub [u32; leaf::N],
    bounds: pub [u64; leaf::N],
    values: [u64; leaf::N],
    hs: [Field; leaf::N],
) {
    for i in 0..leaf::N {
        assert_eq(h2(TYPE_CODE_LITERAL, hs[i]), operands[i], "operand encoding mismatch");
        assert(leaf::verdict(ops[i], bounds[i], values[i]), "filter verdict mismatch");
    }
}
