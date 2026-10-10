// The same variable-length BLAKE3 opening as blake3_leaf.py, written in Noir.
use sparq_zk_compose_core::hashes::{h2, TYPE_CODE_LITERAL};

fn main(
    operands: pub [Field; leaf::N],
    ops: pub [u32; leaf::N],
    bounds: pub [u64; leaf::N],
    values: [u64; leaf::N],
) {
    for i in 0..leaf::N {
        let hs = leaf::literal_hash(values[i]);
        assert_eq(h2(TYPE_CODE_LITERAL, hs), operands[i], "operand encoding mismatch");
        assert(leaf::verdict(ops[i], bounds[i], values[i]), "filter verdict mismatch");
    }
}
