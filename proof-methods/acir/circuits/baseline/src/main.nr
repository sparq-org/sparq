// The Noir opening as the result circuits use it: one fixed-length BLAKE3
// per candidate digit count, all 20 constrained, for each of N values.
use sparq_zk_compose_core::result_integer::full_u64_check;

fn main(
    operands: pub [Field; leaf::N],
    ops: pub [u32; leaf::N],
    bounds: pub [u64; leaf::N],
    values: [u64; leaf::N],
) {
    for i in 0..leaf::N {
        full_u64_check(operands[i], ops[i], bounds[i], values[i]);
    }
}
